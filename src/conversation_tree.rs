use std::collections::HashMap;
use crate::error::RuChatError;

#[derive(Debug, Clone)]
struct Answer {
    id: usize,
    text: Vec<String>,
    response_counter: usize,
    parent_question_id: usize,
    next_edited_answer_id: Option<usize>,
    prev_edited_answer_id: Option<usize>,
}

#[derive(Debug, Clone)]
struct Question {
    id: usize,
    text: Vec<String>,
    answers: Vec<usize>, // Store answer IDs
    current_answer_id: usize,
    parent_question_id: Option<usize>,
    children_question_ids: Vec<usize>,
    next_edited_question_id: Option<usize>,
    prev_edited_question_id: Option<usize>,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationTree {
    questions: HashMap<usize, Question>,
    answers: HashMap<usize, Answer>,
    current_question_ids: Vec<usize>,
    at_question: usize,
    next_question_id: usize,
    next_answer_id: usize,
}

impl ConversationTree {
    pub(crate) fn new() -> Self {
        ConversationTree {
            questions: HashMap::new(),
            answers: HashMap::new(),
            current_question_ids: vec![0],
            at_question: 0,
            next_question_id: 0,
            next_answer_id: 0,
        }
    }

    pub(crate) fn add_question(&mut self, question_text: Vec<String>) -> usize {
        let question_id = self.next_question_id;
        let answer_id = self.next_answer_id;

        let question = Question {
            id: question_id,
            text: question_text,
            answers: vec![answer_id],
            current_answer_id: answer_id,
            parent_question_id: None,
            children_question_ids: Vec::new(),
            next_edited_question_id: None,
            prev_edited_question_id: None,
        };

        // Create a default answer
        let answer = Answer {
            id: answer_id,
            text: Vec::new(),
            response_counter: 0,
            parent_question_id: question_id,
            next_edited_answer_id: None,
            prev_edited_answer_id: None,
        };

        self.questions.insert(question_id, question);
        self.answers.insert(answer_id, answer);

        self.current_question_ids.push(question_id);
        self.at_question += 1;
        self.next_question_id += 1;
        question_id
    }

    pub(crate) fn edit_question(
        &mut self,
        question_id: usize,
        new_question_text: Vec<String>,
    ) -> Result<usize, RuChatError> {
        let question = self.questions.get_mut(&question_id).ok_or(RuChatError::QuestionNotFound)?;

        let new_question_id = self.next_question_id;
        let new_answer_id = self.next_answer_id;
        question.next_edited_question_id = Some(new_question_id);

        let new_question = Question {
            id: new_question_id,
            text: new_question_text,
            answers: vec![new_answer_id],
            current_answer_id: new_answer_id,
            parent_question_id: Some(question_id),
            children_question_ids: Vec::new(),
            next_edited_question_id: None,
            prev_edited_question_id: Some(question_id),
        };

        let new_answer = Answer {
            id: new_answer_id,
            text: Vec::new(),
            response_counter: 0,
            parent_question_id: new_question_id,
            next_edited_answer_id: None,
            prev_edited_answer_id: None,
        };

        self.questions.insert(new_question_id, new_question);
        self.answers.insert(new_answer_id, new_answer);

        if let Some(parent_qid) = self.find_parent(question_id) {
            let parent_question = self.questions.get_mut(&parent_qid).ok_or(RuChatError::QuestionNotFound)?;
            parent_question.children_question_ids.push(new_question_id);
        }
        self.current_question_ids[self.at_question] = new_question_id;
        self.next_question_id += 1;
        Ok(new_question_id)
    }


    pub(crate) fn answer(&mut self, question_id: usize, answer_text: Vec<String>) -> Result<(), RuChatError> {
        let question = self.questions.get_mut(&question_id).ok_or(RuChatError::QuestionNotFound)?;
        let answer_id = question.current_answer_id;
        let answer = self.answers.get_mut(&answer_id).ok_or(RuChatError::AnswerNotFound)?;
        answer.text = answer_text;
        answer.response_counter += 1;
        self.next_answer_id += 1;
        Ok(())
    }

    pub(crate) fn add_answer(&mut self, question_id: usize, new_answer_text: Vec<String>) -> Result<(), RuChatError> {
        let question = self.questions.get_mut(&question_id).ok_or(RuChatError::QuestionNotFound)?;
        let new_answer_id = self.next_answer_id;

        let new_answer = Answer {
            id: new_answer_id,
            text: new_answer_text,
            response_counter: question.answers.len() + 1,
            parent_question_id: question_id,
            next_edited_answer_id: None,
            prev_edited_answer_id: Some(question.current_answer_id),
        };

        question.answers.push(new_answer_id);
        question.current_answer_id = new_answer_id;
        self.answers.insert(new_answer_id, new_answer);

        self.next_answer_id += 1;
        Ok(())
    }

    pub(crate) fn get_question_nr_of_total(&self, question_id: usize) -> String {
        if let Some(question) = self.questions.get(&question_id) {
            let mut prev_next = question.prev_edited_question_id;
            let mut count = 1;
            while let Some(q) = prev_next.and_then(|prev| self.questions.get(&prev)) {
                count += 1;
                prev_next = q.prev_edited_question_id;
            }
            let mut ret = String::from("[");
            if count > 1 {
                ret.push('▲');
            }
            ret.push_str(count.to_string().as_str());
            ret.push('/');

            let mut total = count;
            prev_next = question.next_edited_question_id;
            while let Some(q) = prev_next.and_then(|prev| self.questions.get(&prev)) {
                total += 1;
                prev_next = q.next_edited_question_id;
            }
            if total > count {
                ret.push('▼');
            }
            ret.push_str(total.to_string().as_str());
            ret.push(']');
            ret
        } else {
            String::from("[0/0]")
        }
    }

    pub(crate) fn get_answer_nr_of_total(&self, answer_id: usize) -> String {
        if let Some(answer) = self.answers.get(&answer_id) {
            let mut prev_next = answer.prev_edited_answer_id;
            let mut count = 1;
            while let Some(q) = prev_next.and_then(|prev| self.answers.get(&prev)) {
                count += 1;
                prev_next = q.prev_edited_answer_id;
            }
            let mut ret = String::from("[");
            if count > 1 {
                ret.push('▲');
            }
            ret.push_str(count.to_string().as_str());
            ret.push('/');

            let mut total = count;
            prev_next = answer.next_edited_answer_id;
            while let Some(q) = prev_next.and_then(|prev| self.answers.get(&prev)) {
                total += 1;
                prev_next = q.next_edited_answer_id;
            }
            if total > count {
                ret.push('▼');
            }
            ret.push_str(total.to_string().as_str());
            ret.push(']');
            ret
        } else {
            String::from("[0/0]")
        }
    }

    pub(crate) fn find_parent(&self, question_id: usize) -> Option<usize> {
        self.questions.get(&question_id).and_then(|q| q.parent_question_id)
    }

    pub(crate) fn get_qa(
        &self,
        question_id: usize,
        answer_id: usize,
    ) -> Option<(Vec<String>, Vec<String>)> {
        if let Some(question) = self.questions.get(&question_id) {
            let question_text = question.text.clone();
            let response_text = self
                .answers
                .get(&answer_id)
                .map_or(Vec::new(), |answer| answer.text.clone());
            Some((question_text, response_text))
        } else {
            None
        }
    }

    pub(crate) fn get_current_question_id(&self) -> usize {
        self.current_question_ids[self.at_question]
    }

    pub(crate) fn get_current_answer_id(&self, current_question_id: usize) -> usize {
        self.questions
            .get(&current_question_id)
            .map_or(0, |question| question.current_answer_id)
    }
}
