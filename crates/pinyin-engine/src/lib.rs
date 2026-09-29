#![forbid(unsafe_code)]

//! Small, replaceable local pinyin engine used to bring up the TSF host path.
//!
//! The engine owns composition text and candidate choice. The TSF layer only
//! supplies keys and applies the returned preedit/commit in edit sessions.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineKey {
    Character(char),
    Backspace,
    Space,
    Enter,
    Escape,
    Candidate(usize),
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Preedit {
    Keep,
    Show(String),
    Hide,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EngineReply {
    pub consumed: bool,
    pub preedit: Preedit,
    pub commit: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PinyinEngine {
    input: String,
}

impl PinyinEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) -> EngineReply {
        self.input.clear();
        EngineReply {
            consumed: false,
            preedit: Preedit::Hide,
            commit: None,
        }
    }

    pub fn handle(&mut self, key: EngineKey) -> EngineReply {
        match key {
            EngineKey::Character(character) if character.is_ascii_alphabetic() => {
                if self.input.len() < 32 {
                    self.input.push(character.to_ascii_lowercase());
                }
                self.show_reply()
            }
            EngineKey::Backspace if !self.input.is_empty() => {
                self.input.pop();
                if self.input.is_empty() {
                    EngineReply {
                        consumed: true,
                        preedit: Preedit::Hide,
                        commit: None,
                    }
                } else {
                    self.show_reply()
                }
            }
            EngineKey::Space | EngineKey::Enter if !self.input.is_empty() => {
                self.commit_candidate(0)
            }
            EngineKey::Candidate(index) if !self.input.is_empty() => {
                if index < candidates(&self.input).len() {
                    self.commit_candidate(index)
                } else {
                    self.show_reply()
                }
            }
            EngineKey::Escape if !self.input.is_empty() => {
                self.input.clear();
                EngineReply {
                    consumed: true,
                    preedit: Preedit::Hide,
                    commit: None,
                }
            }
            _ => EngineReply {
                consumed: false,
                preedit: Preedit::Keep,
                commit: None,
            },
        }
    }

    pub fn has_composition(&self) -> bool {
        !self.input.is_empty()
    }

    fn show_reply(&self) -> EngineReply {
        let choices = candidates(&self.input);
        let mut preedit = self.input.clone();
        for (index, candidate) in choices.iter().take(9).enumerate() {
            preedit.push_str(if index == 0 { "  " } else { "   " });
            preedit.push(char::from(b'1' + index as u8));
            preedit.push(':');
            preedit.push_str(candidate);
        }
        EngineReply {
            consumed: true,
            preedit: Preedit::Show(preedit),
            commit: None,
        }
    }

    fn commit_candidate(&mut self, index: usize) -> EngineReply {
        let choices = candidates(&self.input);
        let commit = choices.get(index).copied().unwrap_or(self.input.as_str());
        let commit = commit.to_owned();
        self.input.clear();
        EngineReply {
            consumed: true,
            preedit: Preedit::Hide,
            commit: Some(commit),
        }
    }
}

fn candidates(input: &str) -> &'static [&'static str] {
    match input {
        "a" => &["啊", "阿", "吖"],
        "ai" => &["爱", "矮", "哎"],
        "an" => &["安", "按", "暗"],
        "ba" => &["把", "八", "吧"],
        "bei" => &["被", "北", "杯"],
        "bu" => &["不", "部", "步"],
        "chi" => &["吃", "迟", "持"],
        "de" => &["的", "得", "地"],
        "dian" => &["点", "电", "店"],
        "dui" => &["对", "队", "堆"],
        "hao" => &["好", "号", "浩"],
        "hen" => &["很", "狠", "恨"],
        "jia" => &["家", "加", "价"],
        "jian" => &["见", "件", "间"],
        "jie" => &["界", "接", "借"],
        "jin" => &["今", "进", "近"],
        "kai" => &["开", "看", "凯"],
        "ke" => &["可", "课", "客"],
        "lai" => &["来", "莱", "赖"],
        "le" => &["了", "乐", "勒"],
        "li" => &["里", "理", "力"],
        "ma" => &["吗", "妈", "马"],
        "ming" => &["明", "名", "明天"],
        "mei" => &["没", "美", "每"],
        "ni" => &["你", "呢", "尼"],
        "peng" => &["朋", "碰", "棚"],
        "qi" => &["起", "其", "期"],
        "qing" => &["请", "情", "清"],
        "ren" => &["人", "任", "认"],
        "shi" => &["是", "时", "事", "市"],
        "ta" => &["他", "她", "它"],
        "tian" => &["天", "田", "甜"],
        "wei" => &["为", "位", "未"],
        "wo" => &["我", "握", "窝"],
        "xie" => &["些", "写", "谢"],
        "xue" => &["学", "雪", "血"],
        "yao" => &["要", "摇", "药"],
        "yi" => &["一", "以", "已"],
        "you" => &["有", "又", "友"],
        "zai" => &["在", "再", "载"],
        "zhong" => &["中", "种", "重"],
        "guo" => &["国", "过", "果"],
        "nihao" => &["你好"],
        "zaijian" => &["再见"],
        "xiexie" => &["谢谢"],
        "women" => &["我们"],
        "nimen" => &["你们"],
        "tamen" => &["他们", "她们"],
        "zhongguo" => &["中国"],
        "zhongwen" => &["中文"],
        "hanzi" => &["汉字"],
        "pengyou" => &["朋友"],
        "xuexi" => &["学习"],
        "xuexiao" => &["学校"],
        "laoshi" => &["老师"],
        "xiansheng" => &["先生"],
        "nühao" => &["你好"],
        "woai" => &["我爱"],
        "woaini" => &["我爱你"],
        "duibuqi" => &["对不起"],
        "meiguanxi" => &["没关系"],
        "zaoshanghao" => &["早上好"],
        "wanshanghao" => &["晚上好"],
        "xiexieni" => &["谢谢你"],
        "qingwen" => &["请问"],
        "keyi" => &["可以"],
        "bukeqi" => &["不客气"],
        "zaijianle" => &["再见了"],
        "shenme" => &["什么"],
        "weishenme" => &["为什么"],
        "yinwei" => &["因为"],
        "suoyi" => &["所以"],
        "ruguo" => &["如果"],
        "keshi" => &["可是"],
        "haode" => &["好的"],
        "meiyou" => &["没有"],
        "buyao" => &["不要"],
        "shijie" => &["世界"],
        "shijian" => &["时间"],
        "beijing" => &["北京"],
        "shanghai" => &["上海"],
        "diannao" => &["电脑"],
        "shouji" => &["手机"],
        "gongzuo" => &["工作"],
        "jintian" => &["今天"],
        "mingtian" => &["明天"],
        "zuotian" => &["昨天"],
        "xianzai" => &["现在"],
        "zhidao" => &["知道"],
        "woshi" => &["我是"],
        "woxiang" => &["我想"],
        "woya" => &["我要"],
        "zhege" => &["这个"],
        "nage" => &["那个"],
        "nali" => &["哪里"],
        "zenme" => &["怎么"],
        "duoshao" => &["多少"],
        "yidian" => &["一点"],
        "kaixin" => &["开心"],
        "haochi" => &["好吃"],
        "haokan" => &["好看"],
        "gongsi" => &["公司"],
        "mingzi" => &["名字"],
        "renmin" => &["人民"],
        "chongxin" => &["重新"],
        "qingchu" => &["清楚"],
        "duibu" => &["对不"],
        _ => &[],
    }
}
